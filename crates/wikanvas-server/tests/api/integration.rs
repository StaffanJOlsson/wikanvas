//! Cross-workstream flows (owner: W7)
//!
//! W0b: one flow over the minimal routes — an admin token mints a write token, which creates a collection; revoking
//! the parent revokes the child (D42).
//!
//! W7 (PLAN W7 "Integration tests"): the flows that need several workstreams' full behaviour, end to end over HTTP.
//! The background purge and GC run as the 6-hourly job runs them ([`jobs::purge_and_gc`]) on the injected clock
//! ([`TestApp::with_fake_clock`], `state.clock.advance`), so the 30-day Trash retention and the 24-hour attachment
//! grace (D36) pass without waiting. Clients that must outlive a clock jump use a token without expiry or a "keep me
//! signed in" session made by `POST /auth/login` on that clock.
//!
//! Two rules of §5 shape the GC flows: a Yjs revision records the attachment ids of its state, and a board revision
//! its assets, and both are GC roots. A document's first save writes its `auto` revision; later saves within 10
//! minutes write none, and a token's later writes write no `pre-api` revision either. So each flow saves once before
//! the image arrives, and the image is then referenced only through `item_attachments`.

use std::time::Duration;

use axum::http::StatusCode;
use axum_test::TestServer;
use base64::Engine as _;
use inkboard_core::geom::Frame;
use inkboard_core::model::{BoxGeom, Document, ImageObj, Kind, Label, Object, Page};
use inkboard_core::serial;
use inkboard_core::style::{Align, Style};
use serde_json::{Value, json};
use sha2::Digest as _;
use uuid::Uuid;
use wikanvas_api::{
    Attachment, BoardPutResult, Collection, CollectionTree, CreatedToken, DocState,
    DocUpdateResult, DocWriteResult, ErrorCode, ItemDetail, ItemKind, ItemSummary, ItemText,
    Page as ApiPage, Scope, SearchHit, Sidebar, YFIELD,
};
use wikanvas_core::convert::yrs;
use wikanvas_server::jobs::{self, PurgeReport};
use yrs::updates::decoder::Decode;
use yrs::updates::encoder::Encode;
use yrs::{
    Doc, Options, ReadTxn, Transact, Update, Xml, XmlElementPrelim, XmlFragment, XmlTextPrelim,
};

use crate::common::{OWNER_PASSWORD, TestApp, assert_error, cookie_from};
use crate::files::png;

#[tokio::test]
async fn minted_token_creates_collection_and_dies_with_its_parent() {
    let app = TestApp::with_token(Scope::Admin).await;
    let res = app
        .server
        .post("/api/v1/tokens")
        .json(&json!({"name": "agent", "scopes": ["write"], "expires_in_days": 1}))
        .await;
    res.assert_status(axum::http::StatusCode::CREATED);
    let child: CreatedToken = res.json();
    let agent = app.token_client(&child.token);

    let res = agent
        .post("/api/v1/collections")
        .json(&json!({"name": "Notes", "color": "#336699"}))
        .await;
    res.assert_status(axum::http::StatusCode::CREATED);
    let collection: Collection = res.json();
    assert_eq!(collection.name, "Notes");
    assert_eq!(collection.color, "#336699");
    assert_eq!(collection.item_count, 0);

    app.server
        .delete("/api/v1/tokens/current")
        .await
        .assert_status_no_content();
    let res = agent.get("/api/v1/auth/me").await;
    assert_error(&res, 401, ErrorCode::InvalidToken);
}

// ---------------------------------------------------------------- helpers

/// `POST /collections`.
async fn new_collection(server: &TestServer, name: &str) -> Collection {
    let res = server
        .post("/api/v1/collections")
        .json(&json!({ "name": name }))
        .await;
    res.assert_status(StatusCode::CREATED);
    res.json()
}

/// `POST /items` at the top level of `collection`.
async fn new_item(
    server: &TestServer,
    collection: &Collection,
    kind: &str,
    title: &str,
) -> ItemSummary {
    let res = server
        .post("/api/v1/items")
        .json(&json!({ "collection_id": collection.id, "kind": kind, "title": title }))
        .await;
    res.assert_status(StatusCode::CREATED);
    res.json()
}

/// `GET /items/{id}`.
async fn detail(server: &TestServer, id: Uuid) -> ItemDetail {
    let res = server.get(&format!("/api/v1/items/{id}")).await;
    res.assert_status_ok();
    res.json()
}

/// `GET /items/{id}/text`.
async fn text(server: &TestServer, id: Uuid) -> ItemText {
    let res = server.get(&format!("/api/v1/items/{id}/text")).await;
    res.assert_status_ok();
    res.json()
}

/// `GET /search?{query}`: the hits.
async fn search(server: &TestServer, query: &str) -> Vec<SearchHit> {
    let res = server.get(&format!("/api/v1/search?{query}")).await;
    res.assert_status_ok();
    res.json::<ApiPage<SearchHit>>().items
}

/// The item ids of `search(query)`, in order.
async fn search_ids(server: &TestServer, query: &str) -> Vec<Uuid> {
    search(server, query)
        .await
        .iter()
        .map(|h| h.item.id)
        .collect()
}

/// The marked parts of a hit's snippet.
fn hit_terms(hit: &SearchHit) -> Vec<String> {
    hit.snippet
        .iter()
        .filter(|p| p.hit)
        .map(|p| p.text.clone())
        .collect()
}

/// The current `ETag` of a GET.
async fn etag(server: &TestServer, path: &str) -> String {
    let res = server.get(path).await;
    res.assert_status_ok();
    res.header("etag").to_str().unwrap().to_owned()
}

/// `PUT /items/{id}/doc/markdown` with the document's current ETag.
async fn put_markdown(server: &TestServer, id: Uuid, md: &str) -> DocWriteResult {
    let path = format!("/api/v1/items/{id}/doc/markdown");
    let tag = etag(server, &path).await;
    let res = server
        .put(&path)
        .add_header("if-match", tag)
        .content_type("text/markdown")
        .bytes(md.to_owned().into_bytes().into())
        .await;
    res.assert_status_ok();
    let result: DocWriteResult = res.json();
    assert!(result.changed, "{md:?} changes the document");
    result
}

/// `GET /items/{id}/doc/markdown`.
async fn get_markdown(server: &TestServer, id: Uuid) -> String {
    let res = server
        .get(&format!("/api/v1/items/{id}/doc/markdown"))
        .await;
    res.assert_status_ok();
    res.text()
}

/// `PUT /items/{id}/board` with the board's current ETag.
async fn put_board(server: &TestServer, id: Uuid, doc: &Document) -> BoardPutResult {
    let path = format!("/api/v1/items/{id}/board");
    let tag = etag(server, &path).await;
    let res = server
        .put(&path)
        .add_header("if-match", tag)
        .text(serial::to_json(doc, false))
        .content_type("application/json")
        .await;
    res.assert_status_ok();
    res.json()
}

/// `POST /attachments` (a raw body, as every client sends it).
async fn upload(server: &TestServer, item: Uuid, filename: &str, bytes: Vec<u8>) -> Attachment {
    let res = server
        .post(&format!(
            "/api/v1/attachments?item_id={item}&filename={filename}"
        ))
        .bytes(bytes.into())
        .content_type("application/octet-stream")
        .await;
    res.assert_status(StatusCode::CREATED);
    res.json()
}

/// The status of `GET /attachments/{id}`, and its body.
async fn download(server: &TestServer, id: Uuid) -> (u16, Vec<u8>) {
    let res = server.get(&format!("/api/v1/attachments/{id}")).await;
    (res.status_code().as_u16(), res.as_bytes().to_vec())
}

/// A box with a label.
fn label_box(text: &str, x: f64) -> Object {
    Object::new_box(
        BoxGeom::Rect,
        Frame::new(x, 0.0, 200.0, 80.0),
        Style::default(),
        Label::new(text, 20.0, Align::Center),
    )
}

/// An image object showing `asset`.
fn image(asset: inkboard_core::model::AssetId, x: f64) -> Object {
    Object::new(
        Kind::Image(ImageObj {
            frame: Frame::new(x, 200.0, 40.0, 40.0),
            asset,
            latex: None,
            mono: false,
            recolor: Vec::new(),
        }),
        Style::default(),
    )
}

/// Lowercase hex sha256, as the blob store names files.
fn sha256_hex(bytes: &[u8]) -> String {
    sha2::Sha256::digest(bytes)
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect()
}

/// The blob file of `sha` (`files/aa/bb/<sha256>`, D26).
fn blob_file(app: &TestApp, sha: &str) -> std::path::PathBuf {
    app.state
        .config
        .files_dir()
        .join(&sha[0..2])
        .join(&sha[2..4])
        .join(sha)
}

/// Board assets of an item.
const ITEM_ASSETS: &str = "SELECT count(*) FROM board_assets WHERE item_id = ?1";
/// Board assets that use a blob.
const BLOB_USES: &str = "SELECT count(*) FROM board_assets WHERE sha256 = ?1";
/// The blob row of a hash.
const BLOB_ROWS: &str = "SELECT count(*) FROM blobs WHERE sha256 = ?1";
/// Revisions of an item.
const ITEM_REVISIONS: &str = "SELECT count(*) FROM revisions WHERE item_id = ?1";

/// `SELECT count(*) …` with one text parameter.
async fn count(app: &TestApp, sql: &'static str, param: impl ToString) -> i64 {
    let param = param.to_string();
    app.state
        .db
        .read(move |c| Ok(c.query_row(sql, [&param], |r| r.get(0))?))
        .await
        .expect("count")
}

/// The attachment ids `item_attachments` holds for `item`.
async fn item_attachments(app: &TestApp, item: Uuid) -> Vec<String> {
    let item = item.to_string();
    app.state
        .db
        .read(move |c| {
            Ok(c.prepare(
                "SELECT attachment_id FROM item_attachments WHERE item_id = ?1 ORDER BY attachment_id",
            )?
            .query_map([&item], |r| r.get(0))?
            .collect::<rusqlite::Result<Vec<String>>>()?)
        })
        .await
        .expect("item_attachments")
}

/// How many revisions hold `attachment` among their GC roots.
async fn revisions_holding(app: &TestApp, attachment: Uuid) -> i64 {
    count(
        app,
        "SELECT count(*) FROM revisions r, json_each(r.attachment_ids) j
         WHERE r.attachment_ids IS NOT NULL AND j.value = ?1",
        attachment,
    )
    .await
}

/// The owner and a "keep me signed in" session from `POST /auth/login` on the app's clock (30 days), as the default
/// client.
async fn sign_in_remembered(app: &mut TestApp) {
    let owner = app.insert_owner().await;
    let res = app.login(OWNER_PASSWORD, true, None).await;
    res.assert_status_ok();
    let session = cookie_from(&res, "wikanvas_session").expect("a session cookie");
    app.server = app.cookie_client(&session);
    app.owner_id = Some(owner);
    app.session = Some(session);
}

/// One purge-and-GC run of the background job.
async fn purge_and_gc(app: &TestApp) -> PurgeReport {
    jobs::purge_and_gc(&app.state).await.expect("purge and GC")
}

/// A Y client of the document routes, as the web editor is (UTF-16 offsets, the `default` fragment).
struct YClient {
    doc: Doc,
}

impl YClient {
    fn new(id: u64) -> YClient {
        YClient {
            doc: Doc::with_options(Options {
                offset_kind: yrs::OffsetKind::Utf16,
                ..Options::with_client_id(yrs::ClientID::new(id))
            }),
        }
    }

    /// `GET …/doc?since=` and applies what it lacks.
    async fn sync(&self, server: &TestServer, id: Uuid) {
        let b64 = base64::engine::general_purpose::STANDARD;
        let sv = b64.encode(self.doc.transact().state_vector().encode_v1());
        let res = server
            .get(&format!("/api/v1/items/{id}/doc"))
            .add_query_param("since", sv)
            .await;
        res.assert_status_ok();
        let state: DocState = res.json();
        let update = Update::decode_v1(&b64.decode(&state.update).unwrap()).unwrap();
        self.doc.transact_mut().apply_update(update).unwrap();
    }

    /// Applies `edit` locally and `POST`s its update to `…/doc/updates`.
    async fn edit(
        &self,
        server: &TestServer,
        id: Uuid,
        edit: impl FnOnce(&mut yrs::TransactionMut, &yrs::XmlFragmentRef),
    ) -> DocUpdateResult {
        let frag = self.doc.get_or_insert_xml_fragment(YFIELD);
        let before = self.doc.transact().state_vector();
        edit(&mut self.doc.transact_mut(), &frag);
        let update = self.doc.transact().encode_diff_v1(&before);
        let res = server
            .post(&format!("/api/v1/items/{id}/doc/updates"))
            .json(&json!({"update": base64::engine::general_purpose::STANDARD.encode(update)}))
            .await;
        res.assert_status_ok();
        res.json()
    }
}

// ---------------------------------------------------------------- the flows

/// W1 purge, W2 GC, W6 board assets. Two boards share one image (one blob, deduplicated by hash) and one has a
/// second image of its own. A trashed board keeps its blobs; purging it over HTTP frees only the blob nothing else
/// uses, and the 30-day retention purge of the other board frees the shared one.
#[tokio::test]
async fn board_blobs_freed_after_purge_and_gc() {
    let mut app = TestApp::with_fake_clock().await;
    app.sign_in_owner().await;
    let admin = app.token_client(&app.mint_token(app.owner(), Scope::Admin).await);
    let c = new_collection(&app.server, "Boards").await;
    let a = new_item(&app.server, &c, "board", "A").await;
    let b = new_item(&app.server, &c, "board", "B").await;
    let (shared, own) = (png(2, 2), png(3, 3));
    let (shared_sha, own_sha) = (sha256_hex(&shared), sha256_hex(&own));

    let mut board_a = Document::new("A");
    let s = board_a.add_asset("image/png", shared.clone());
    let o = board_a.add_asset("image/png", own.clone());
    board_a.pages[0]
        .objects
        .extend([image(s, 0.0), image(o, 100.0), label_box("A's images", 0.0)]);
    put_board(&app.server, a.id, &board_a).await;
    let mut board_b = Document::new("B");
    let s = board_b.add_asset("image/png", shared.clone());
    board_b.pages[0].objects.push(image(s, 0.0));
    put_board(&app.server, b.id, &board_b).await;

    assert_eq!(count(&app, ITEM_ASSETS, a.id).await, 2);
    assert_eq!(
        count(&app, BLOB_USES, &shared_sha).await,
        2,
        "both boards use the shared image"
    );
    assert_eq!(
        count(&app, BLOB_ROWS, &shared_sha).await,
        1,
        "one blob for the shared image"
    );
    assert!(blob_file(&app, &shared_sha).exists());
    assert!(blob_file(&app, &own_sha).exists());
    assert_eq!(purge_and_gc(&app).await, PurgeReport::default());

    // In the Trash, A still holds its blobs: a restore must find them.
    app.server
        .delete(&format!("/api/v1/items/{}", a.id))
        .await
        .assert_status_ok();
    assert_eq!(purge_and_gc(&app).await, PurgeReport::default());
    assert!(blob_file(&app, &own_sha).exists());

    // Purged over HTTP: the next GC frees A's own image and keeps the one B still shows.
    admin
        .delete(&format!("/api/v1/trash/items/{}", a.id))
        .await
        .assert_status(StatusCode::NO_CONTENT);
    assert_error(
        &admin.get(&format!("/api/v1/items/{}", a.id)).await,
        404,
        ErrorCode::NotFound,
    );
    assert_eq!(count(&app, ITEM_ASSETS, a.id).await, 0);
    assert_eq!(
        count(&app, ITEM_REVISIONS, a.id).await,
        0,
        "the purge takes A's revisions, which held its assets"
    );
    let report = purge_and_gc(&app).await;
    assert_eq!((report.items_purged, report.blobs.blobs), (0, 1));
    assert_eq!(count(&app, BLOB_ROWS, &own_sha).await, 0);
    assert!(
        !blob_file(&app, &own_sha).exists(),
        "A's own image is unlinked"
    );
    assert!(
        blob_file(&app, &shared_sha).exists(),
        "B still uses the shared image"
    );
    let res = app
        .server
        .get(&format!("/api/v1/items/{}/board", b.id))
        .await;
    res.assert_status_ok();
    let assets = &res.json::<Value>()["document"]["assets"];
    assert_eq!(assets.as_object().map(|m| m.len()), Some(1), "{assets}");
    let data = assets.as_object().unwrap().values().next().unwrap()["data"]
        .as_str()
        .unwrap()
        .to_owned();
    assert_eq!(
        base64::engine::general_purpose::STANDARD
            .decode(data)
            .unwrap(),
        shared,
        "B's image bytes come back from the blob"
    );

    // B goes to the Trash; 31 days later the scheduled run purges it, then frees the shared blob.
    app.server
        .delete(&format!("/api/v1/items/{}", b.id))
        .await
        .assert_status_ok();
    assert_eq!(purge_and_gc(&app).await, PurgeReport::default());
    app.state.clock.advance(Duration::from_secs(31 * 86_400));
    let report = purge_and_gc(&app).await;
    assert_eq!(
        (
            report.items_purged,
            report.attachments_deleted,
            report.blobs.blobs
        ),
        (1, 0, 1)
    );
    assert_eq!(count(&app, BLOB_ROWS, &shared_sha).await, 0);
    assert!(!blob_file(&app, &shared_sha).exists());
    assert_error(
        &admin.get(&format!("/api/v1/items/{}", b.id)).await,
        404,
        ErrorCode::NotFound,
    );
}

/// W1 search, W6 board text. Labels on every page and the page names of a board PUT are found by `GET /search`,
/// with the hit marked in the snippet; a later PUT replaces them in the index.
#[tokio::test]
async fn search_finds_board_label_over_http() {
    let app = TestApp::with_owner().await;
    let c = new_collection(&app.server, "Field work").await;
    let board = new_item(&app.server, &c, "board", "Sketch").await;
    let doc = new_item(&app.server, &c, "doc", "Unrelated").await;
    let mut d = Document::new("Sketch");
    d.pages[0].objects.push(label_box("Quokka habitat", 0.0));
    let mut second = Page::new("Roadmap");
    second.objects.push(Object::new_box(
        BoxGeom::Sticky,
        Frame::new(0.0, 0.0, 200.0, 200.0),
        Style::default(),
        Label::new("Numbat survey in spring", 18.0, Align::Left),
    ));
    d.pages.push(second);
    put_board(&app.server, board.id, &d).await;

    let hits = search(&app.server, "q=quok").await;
    assert_eq!(
        hits.iter().map(|h| h.item.id).collect::<Vec<_>>(),
        [board.id]
    );
    assert_eq!(hits[0].item.kind, ItemKind::Board);
    assert_eq!(hit_terms(&hits[0]), ["Quokka"]);
    let snippet: String = hits[0].snippet.iter().map(|p| p.text.as_str()).collect();
    assert!(snippet.contains("Quokka habitat"), "{snippet}");
    // A label on the second page, a sticky note's label, and a page name.
    assert_eq!(
        search_ids(&app.server, "q=numbat%20spring").await,
        [board.id]
    );
    assert_eq!(search_ids(&app.server, "q=roadmap").await, [board.id]);
    // The kind filter, and a document that does not match.
    assert_eq!(
        search_ids(&app.server, "q=quokka&kind=board").await,
        [board.id]
    );
    assert!(
        search_ids(&app.server, "q=quokka&kind=doc")
            .await
            .is_empty()
    );
    assert!(!search_ids(&app.server, "q=quokka").await.contains(&doc.id));

    // A new label replaces the old one in the index.
    d.pages[0].objects.clear();
    d.pages[0].objects.push(label_box("Wombat burrow", 0.0));
    put_board(&app.server, board.id, &d).await;
    assert!(search_ids(&app.server, "q=quokka").await.is_empty());
    assert_eq!(search_ids(&app.server, "q=wombat").await, [board.id]);
}

/// W2 seed, W1 search, W4/W6 fixtures. `POST /setup` seeds "Getting started" from the committed fixtures; every
/// seeded item is published and its content is found by search.
#[tokio::test]
async fn seeded_getting_started_is_published_and_searchable() {
    let app = TestApp::new().await;
    let res = app
        .anon_client()
        .post("/api/v1/setup")
        .json(&json!({
            "code": &*app.state.setup_code,
            "workspace_name": "Acme",
            "name": "Ada",
            "email": "ada@wikanvas.invalid",
            "password": "a long enough password"
        }))
        .await;
    res.assert_status(StatusCode::CREATED);
    let owner =
        app.cookie_client(&cookie_from(&res, "wikanvas_session").expect("a session cookie"));

    let res = owner.get("/api/v1/sidebar").await;
    res.assert_status_ok();
    let sidebar: Sidebar = res.json();
    assert_eq!(sidebar.drafts_count, 0, "nothing seeded is a draft");
    assert_eq!(
        sidebar
            .collections
            .iter()
            .map(|c| c.name.as_str())
            .collect::<Vec<_>>(),
        ["Getting started"]
    );
    let res = owner
        .get(&format!(
            "/api/v1/collections/{}/tree",
            sidebar.collections[0].id
        ))
        .await;
    res.assert_status_ok();
    let tree: CollectionTree = res.json();
    let by_title = |title: &str| {
        tree.items
            .iter()
            .find(|i| i.title == title)
            .unwrap_or_else(|| panic!("{title} is seeded: {:?}", tree.items))
            .clone()
    };
    let welcome = by_title("Welcome to Wikanvas");
    let writing = by_title("Writing in Wikanvas");
    let board = by_title("Example board");
    assert_eq!(tree.items.len(), 3);
    assert_eq!(
        (welcome.kind, writing.kind, board.kind),
        (ItemKind::Doc, ItemKind::Doc, ItemKind::Board)
    );
    assert_eq!(welcome.parent_id, None);
    assert_eq!(
        (writing.parent_id, board.parent_id),
        (Some(welcome.id), Some(welcome.id))
    );
    for item in [&welcome, &writing, &board] {
        let d = detail(&owner, item.id).await;
        assert!(
            d.item.published && d.effectively_published,
            "{} is published",
            item.title
        );
    }

    // The content of each fixture is indexed: a word only the welcome text, only "Writing in Wikanvas" and only the
    // example board contain. Published items are found without drafts too.
    for (query, item) in [
        ("q=whiteboards", &welcome),
        ("q=slash%20menu", &writing),
        ("q=shapes", &board),
    ] {
        for filter in ["", "&include_drafts=false"] {
            let hits = search(&owner, &format!("{query}{filter}")).await;
            assert_eq!(
                hits.iter().map(|h| h.item.id).collect::<Vec<_>>(),
                [item.id],
                "{query}{filter} finds {}",
                item.title
            );
            assert!(!hit_terms(&hits[0]).is_empty(), "{query}: a marked snippet");
        }
    }
    // The texts behind them come from the fixtures.
    let welcome_text = text(&owner, welcome.id).await;
    assert!(
        welcome_text
            .body
            .starts_with("Wikanvas keeps your documents and whiteboards in one place."),
        "{}",
        welcome_text.body
    );
    let writing_text = text(&owner, writing.id).await;
    assert!(
        writing_text.body.contains("The slash menu"),
        "{}",
        writing_text.body
    );
    let board_text = text(&owner, board.id).await;
    assert!(
        board_text.body.contains("Welcome to your first board"),
        "{}",
        board_text.body
    );
    assert_eq!(
        detail(&owner, board.id).await.board.map(|m| m.objects > 0),
        Some(true),
        "the example board has its shapes"
    );
}

/// W4 `item_attachments`, W2 attachment GC. An image the editor inserts (a Yjs update over HTTP) is referenced
/// through `item_attachments`; once it is removed it is kept until 24 h after its upload (the grace of D36), then the
/// GC deletes the attachment and its blob.
#[tokio::test]
async fn doc_image_attachment_freed_after_removal() {
    let mut app = TestApp::with_fake_clock().await;
    sign_in_remembered(&mut app).await;
    let web = &app.server;
    let c = new_collection(web, "Trips").await;
    let doc = new_item(web, &c, "doc", "Coast").await;
    let editor = YClient::new(4242);
    editor.sync(web, doc.id).await;
    // The first save (it writes the document's auto revision, before there is an image).
    editor
        .edit(web, doc.id, |txn, frag| {
            let p = frag.insert(txn, 0, XmlElementPrelim::empty("paragraph"));
            p.insert(txn, 0, XmlTextPrelim::new("A photo of the coast:"));
        })
        .await;
    let bytes = png(4, 4);
    let att = upload(web, doc.id, "coast.png", bytes.clone()).await;
    assert_eq!(att.mime, "image/png");
    let id = att.id.to_string();
    editor
        .edit(web, doc.id, |txn, frag| {
            let img = frag.insert(txn, 1, XmlElementPrelim::empty("image"));
            img.insert_attribute(txn, "src", format!("/api/v1/attachments/{id}"));
            img.insert_attribute(txn, "attachmentId", id.clone());
            img.insert_attribute(txn, "alt", "The coast");
        })
        .await;
    assert_eq!(item_attachments(&app, doc.id).await, [att.id.to_string()]);
    assert_eq!(revisions_holding(&app, att.id).await, 0);
    assert!(
        get_markdown(web, doc.id)
            .await
            .contains(&format!("![The coast](/api/v1/attachments/{})", att.id)),
        "the conversion sees the attachment"
    );

    // An hour later the image is deleted from the document.
    app.state.clock.advance(Duration::from_secs(3600));
    editor
        .edit(web, doc.id, |txn, frag| frag.remove_range(txn, 1, 1))
        .await;
    assert!(item_attachments(&app, doc.id).await.is_empty());
    let file = blob_file(&app, &att.sha256);
    // Within 24 h of the upload it is kept.
    assert_eq!(purge_and_gc(&app).await, PurgeReport::default());
    app.state
        .clock
        .advance(Duration::from_secs(22 * 3600 + 58 * 60));
    assert_eq!(purge_and_gc(&app).await, PurgeReport::default());
    assert!(file.exists());
    assert_eq!(download(web, att.id).await, (200, bytes));
    // Past 24 h, the attachment and its blob go.
    app.state.clock.advance(Duration::from_secs(3 * 60));
    let report = purge_and_gc(&app).await;
    assert_eq!((report.attachments_deleted, report.blobs.blobs), (1, 1));
    assert!(!file.exists(), "the blob file is unlinked");
    assert_error(
        &web.get(&format!("/api/v1/attachments/{}", att.id)).await,
        404,
        ErrorCode::NotFound,
    );
    assert_eq!(text(web, doc.id).await.body, "A photo of the coast:");
}

/// W2 tokens, W4 conversion, W1 search. A write token PUTs Markdown; its text, without the Markdown syntax, is what
/// search finds and `/text` returns, and the next PUT replaces it.
#[tokio::test]
async fn markdown_put_with_token_is_searchable() {
    let app = TestApp::with_token(Scope::Write).await;
    let agent = &app.server;
    let c = new_collection(agent, "Agent notes").await;
    let doc = new_item(agent, &c, "doc", "Field notes").await;
    let md = "# Sightings\n\nA **numbat** crossed the _track_ at dawn.\n\n- termite mound\n- fallen log\n";
    let result = put_markdown(agent, doc.id, md).await;
    assert!(result.warnings.is_empty(), "{:?}", result.warnings);

    let hits = search(agent, "q=numb").await;
    assert_eq!(hits.iter().map(|h| h.item.id).collect::<Vec<_>>(), [doc.id]);
    assert_eq!(hit_terms(&hits[0]), ["numbat"]);
    assert_eq!(search_ids(agent, "q=termite%20mound").await, [doc.id]);
    assert_eq!(
        search_ids(agent, "q=%22crossed%20the%20track%22").await,
        [doc.id]
    );
    let body = text(agent, doc.id).await.body;
    assert!(
        body.contains("A numbat crossed the track at dawn."),
        "{body}"
    );
    assert!(!body.contains("**") && !body.contains("# "), "{body}");
    // A read token finds it too, and cannot write.
    let reader = app.token_client(&app.mint_token(app.owner(), Scope::Read).await);
    assert_eq!(search_ids(&reader, "q=numbat").await, [doc.id]);
    let res = reader
        .put(&format!("/api/v1/items/{}/doc/markdown", doc.id))
        .add_header("if-match", "*")
        .content_type("text/markdown")
        .bytes("nope\n".as_bytes().to_vec().into())
        .await;
    assert_error(&res, 403, ErrorCode::InsufficientScope);

    // The next PUT replaces the indexed text.
    put_markdown(agent, doc.id, "A quokka, this time.\n").await;
    assert!(search_ids(agent, "q=numbat").await.is_empty());
    assert_eq!(search_ids(agent, "q=quokka").await, [doc.id]);
}

/// W4 Markdown image resolution, W2 attachment GC. `![alt](/api/v1/attachments/<id>)` in a Markdown PUT becomes an
/// image with that `attachmentId`, so the GC keeps the attachment past its 24 h grace while the document shows it
/// (an attachment uploaded at the same time and never referenced goes); once the reference is gone, it goes too.
#[tokio::test]
async fn markdown_attachment_reference_keeps_attachment_alive() {
    let mut app = TestApp::with_fake_clock().await;
    let owner = app.insert_owner().await;
    let agent = app.token_client(&app.mint_token(&owner, Scope::Write).await);
    app.owner_id = Some(owner);
    let c = new_collection(&agent, "Maps").await;
    let doc = new_item(&agent, &c, "doc", "Site").await;
    put_markdown(&agent, doc.id, "Site map below.\n").await;
    let bytes = png(5, 5);
    let kept = upload(&agent, doc.id, "map.png", bytes.clone()).await;
    let loose = upload(&agent, doc.id, "unused.png", png(6, 6)).await;
    let md = format!(
        "Site map below.\n\n![site map](/api/v1/attachments/{})\n",
        kept.id
    );
    put_markdown(&agent, doc.id, &md).await;
    assert_eq!(
        get_markdown(&agent, doc.id).await,
        md,
        "the image reads back as written"
    );
    let res = agent
        .get(&format!("/api/v1/items/{}/doc/prosemirror", doc.id))
        .await;
    res.assert_status_ok();
    let pm: Value = res.json();
    let images: Vec<&Value> = pm["doc"]["content"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|n| n["type"] == "image")
        .collect();
    assert_eq!(images.len(), 1, "{pm}");
    assert_eq!(images[0]["attrs"]["attachmentId"], kept.id.to_string());
    assert_eq!(item_attachments(&app, doc.id).await, [kept.id.to_string()]);
    assert_eq!(revisions_holding(&app, kept.id).await, 0);

    // Two days later: the referenced attachment stays, the unreferenced one is deleted.
    app.state.clock.advance(Duration::from_secs(48 * 3600));
    let report = purge_and_gc(&app).await;
    assert_eq!((report.attachments_deleted, report.blobs.blobs), (1, 1));
    assert_eq!(download(&agent, kept.id).await, (200, bytes));
    assert!(blob_file(&app, &kept.sha256).exists());
    assert_eq!(download(&agent, loose.id).await.0, 404);
    assert!(!blob_file(&app, &loose.sha256).exists());
    assert_eq!(purge_and_gc(&app).await, PurgeReport::default());

    // Without the reference, the next run deletes it.
    put_markdown(&agent, doc.id, "Site map removed.\n").await;
    assert!(item_attachments(&app, doc.id).await.is_empty());
    let report = purge_and_gc(&app).await;
    assert_eq!((report.attachments_deleted, report.blobs.blobs), (1, 1));
    assert_eq!(download(&agent, kept.id).await.0, 404);
    assert!(!blob_file(&app, &kept.sha256).exists());
}

/// W2 tokens, W6 board PUT and previews, W1 `/items/{id}/text`. A board a write token PUTs is readable by a read
/// token as text, as `ItemDetail.board` meta, and as the SVG previews of its pages.
#[tokio::test]
async fn board_token_put_then_text_and_preview() {
    let app = TestApp::with_owner().await;
    let agent = app.token_client(&app.mint_token(app.owner(), Scope::Write).await);
    let reader = app.token_client(&app.mint_token(app.owner(), Scope::Read).await);
    let c = new_collection(&agent, "Planning").await;
    let board = new_item(&agent, &c, "board", "Plan").await;
    let mut d = Document::new("Plan");
    d.pages[0].objects.extend([
        label_box("Kickoff meeting", 0.0),
        label_box("Ship beta", 300.0),
    ]);
    let mut later = Page::new("Later");
    later.objects.push(label_box("Retrospective", 0.0));
    d.pages.push(later);
    let put = put_board(&agent, board.id, &d).await;
    assert_eq!(put.rev, 2);
    assert_eq!(put.etag, "\"r2\"");

    // Text: the page names and labels, for the search index and agents.
    let t = text(&reader, board.id).await;
    assert_eq!((t.kind, t.title.as_str()), (ItemKind::Board, "Plan"));
    let lines: Vec<&str> = t.body.lines().collect();
    for want in [
        "Page 1",
        "Kickoff meeting",
        "Ship beta",
        "Later",
        "Retrospective",
    ] {
        assert!(lines.contains(&want), "{want:?} in {:?}", t.body);
    }
    // The meta.
    let meta = detail(&reader, board.id).await.board.expect("board meta");
    assert_eq!((meta.rev, meta.objects), (2, 3));
    assert_eq!(
        meta.pages
            .iter()
            .map(|p| p.name.as_str())
            .collect::<Vec<_>>(),
        ["Page 1", "Later"]
    );
    assert!(
        meta.text_excerpt.contains("Kickoff meeting"),
        "{}",
        meta.text_excerpt
    );

    // The previews of both pages, rendered for r2.
    let page = |n: u32| format!("/api/v1/items/{}/board/pages/{n}", board.id);
    let mut svgs = Vec::new();
    for (n, label) in [(0, "Kickoff meeting"), (1, "Retrospective")] {
        let res = reader.get(&page(n)).await;
        res.assert_status_ok();
        assert_eq!(res.content_type(), "image/svg+xml");
        assert_eq!(res.header("etag"), "\"r2\"");
        let svg = res.text();
        assert!(svg.contains("<svg") && svg.contains(label), "{svg}");
        svgs.push(svg);
    }
    assert!(
        !svgs[1].contains("Kickoff meeting"),
        "page 1 shows only its own objects"
    );
    let meta = detail(&reader, board.id).await.board.expect("board meta");
    assert_eq!(
        meta.preview_rev,
        Some(2),
        "the rendered previews are stored"
    );
    assert_eq!(meta.pages[0].svg_len, Some(svgs[0].len() as u64));
    let res = reader.get(&format!("{}?rev=2", page(0))).await;
    res.assert_status_ok();
    assert_eq!(
        res.header("cache-control"),
        "private, max-age=31536000, immutable"
    );
    assert_eq!(res.text(), svgs[0]);
    // The board GET returns what the token wrote.
    let res = reader
        .get(&format!("/api/v1/items/{}/board", board.id))
        .await;
    res.assert_status_ok();
    assert_eq!(res.header("etag"), "\"r2\"");
    let back: Value = res.json();
    assert_eq!(back["document"]["pages"].as_array().map(Vec::len), Some(2));
}

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
        let size = usize::from_str_radix(field(124..136).trim(), 8).unwrap();
        out.push((name, bytes[at + 512..at + 512 + size].to_vec()));
        at += 512 + size.div_ceil(512) * 512;
    }
    out
}

/// W2 export and tokens, W4/W6 writes. After a write token created a document (Markdown with an image) and a board
/// (with an image asset), the export carries that content and its files, and no session, token or password hash.
#[tokio::test]
async fn export_after_api_writes_has_no_tokens_or_sessions() {
    let app = TestApp::with_owner().await;
    let admin_token = app.mint_token(app.owner(), Scope::Admin).await;
    let write_token = app.mint_token(app.owner(), Scope::Write).await;
    let (admin, agent) = (
        app.token_client(&admin_token),
        app.token_client(&write_token),
    );
    let c = new_collection(&agent, "Agent work").await;
    let doc = new_item(&agent, &c, "doc", "Report").await;
    let photo = png(7, 7);
    let att = upload(&agent, doc.id, "photo.png", photo.clone()).await;
    put_markdown(
        &agent,
        doc.id,
        &format!(
            "Export canary: platypus.\n\n![photo](/api/v1/attachments/{})\n",
            att.id
        ),
    )
    .await;
    let board = new_item(&agent, &c, "board", "Board").await;
    let mut d = Document::new("Board");
    let asset_bytes = png(8, 8);
    let asset = d.add_asset("image/png", asset_bytes.clone());
    d.pages[0]
        .objects
        .extend([label_box("Echidna canary", 0.0), image(asset, 300.0)]);
    put_board(&agent, board.id, &d).await;
    // A Yjs save too, as the web editor makes them.
    let editor = YClient::new(77);
    editor.sync(&agent, doc.id).await;
    editor
        .edit(&agent, doc.id, |txn, frag| {
            let n = frag.len(txn);
            let p = frag.insert(txn, n, XmlElementPrelim::empty("paragraph"));
            p.insert(txn, 0, XmlTextPrelim::new("Added over Yjs."));
        })
        .await;

    let res = admin.get("/api/v1/export").await;
    res.assert_status_ok();
    assert_eq!(res.header("content-type"), "application/x-tar");
    let entries = untar(res.as_bytes());
    let names: Vec<&str> = entries.iter().map(|(n, _)| n.as_str()).collect();
    let file_of = |sha: &str| format!("files/{}/{}/{sha}", &sha[0..2], &sha[2..4]);
    assert_eq!(names[..2], ["manifest.json", "wikanvas.db"]);
    for (sha, bytes) in [
        (att.sha256.clone(), &photo),
        (sha256_hex(&asset_bytes), &asset_bytes),
    ] {
        let entry = entries
            .iter()
            .find(|(n, _)| *n == file_of(&sha))
            .unwrap_or_else(|| panic!("{sha} in {names:?}"));
        assert_eq!(&entry.1, bytes);
    }

    let raw = &entries[1].1;
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("wikanvas.db"), raw).unwrap();
    let conn = rusqlite::Connection::open_with_flags(
        dir.path().join("wikanvas.db"),
        rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY,
    )
    .unwrap();
    let n = |sql: &str| -> i64 { conn.query_row(sql, [], |r| r.get(0)).unwrap() };
    assert_eq!(n("SELECT count(*) FROM sessions"), 0);
    assert_eq!(n("SELECT count(*) FROM api_tokens"), 0);
    assert_eq!(n("SELECT count(*) FROM api_token_scopes"), 0);
    assert_eq!(
        n("SELECT count(*) FROM users WHERE password_hash IS NOT NULL"),
        0
    );
    // The API writes are there.
    let body = |id: Uuid| -> String {
        conn.query_row(
            "SELECT body FROM item_text WHERE item_id = ?1",
            [id.to_string()],
            |r| r.get(0),
        )
        .unwrap()
    };
    let doc_body = body(doc.id);
    assert!(
        doc_body.contains("Export canary: platypus.") && doc_body.contains("Added over Yjs."),
        "{doc_body}"
    );
    assert!(body(board.id).contains("Echidna canary"));
    assert_eq!(n("SELECT count(*) FROM item_attachments"), 1);
    assert_eq!(n("SELECT count(*) FROM board_assets"), 1);
    assert_eq!(n("SELECT count(*) FROM doc_updates"), 2);
    // No credential survives in the bytes either.
    let contains = |needle: &[u8]| raw.windows(needle.len()).any(|w| w == needle);
    for token in [&admin_token, &write_token] {
        assert!(!contains(token.as_bytes()), "a token secret");
        assert!(!contains(&token.as_bytes()[..12]), "a token prefix");
        assert!(
            !contains(&wikanvas_store::tokens::hash(token)),
            "a token hash"
        );
    }
    assert!(!contains(b"$argon2id$"), "a password hash");
    let session = wikanvas_store::sessions::token_hash(app.session.as_deref().unwrap()).unwrap();
    assert!(!contains(&session), "a session hash");

    // The live database still has them.
    agent.get("/api/v1/auth/me").await.assert_status_ok();
    admin.get("/api/v1/auth/me").await.assert_status_ok();
    app.server.get("/api/v1/auth/me").await.assert_status_ok();
}
