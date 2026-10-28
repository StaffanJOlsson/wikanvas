//! Board storage tests (owner: W6)
//!
//! The initializers, the PUT pipeline (`prepare` outside the writer, `save` inside it), the GET path (`stored` +
//! `assemble`), previews and `board_meta`, and the seed fixture `fixtures/boards/getting-started.inkboard` (§9.6.7),
//! which [`getting_started`] builds with fixed ids. `WRITE_FIXTURES=1 scripts/dev.sh cargo test -p quire-store
//! --locked --test store -- boards::` rewrites the fixture when it differs from what the code produces.

use std::path::{Path, PathBuf};

use inkboard_core::connector::{Connector, End};
use inkboard_core::geom::Frame;
use inkboard_core::model::{BoxGeom, Document, Id, ImageObj, Kind, Label, Object};
use inkboard_core::serial;
use inkboard_core::style::{Align, Color, FILLS, NOTE_YELLOW, Style};
use quire_core::api::{ErrorCode, ItemKind};
use quire_store::boards::{self, IfMatch, Saved};
use quire_store::{WriteBy, previews};
use rusqlite::params;
use serde_json::Value;

use crate::common::{MINUTE, NOW, TestDb, count};

const PNG: &[u8] =
    b"\x89PNG\r\n\x1a\n\0\0\0\rIHDR\0\0\0\x02\0\0\0\x02\x08\x06\0\0\0\x72\xb6\x0d\x24";
const GIF: &[u8] = b"GIF89a\x01\x00\x01\x00\x80\x00\x00\xff\xff\xff\x00\x00\x00!\xf9\x04\x01\x00\x00\x00\x00,\x00\x00\x00\x00\x01\x00\x01\x00\x00\x02\x02D\x01\x00;";

fn fixture_path() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../fixtures/boards/getting-started.inkboard")
}

/// The "Example board" of the Getting started seed (§9.6.7): a title, three labelled shapes, two connectors and a
/// sticky note, with fixed ids so the file is byte-stable.
fn getting_started() -> String {
    let id = |s: &str| Id::from(s);
    let filled = |fill: Color| Style {
        fill: Some(fill),
        roundness: 0.3,
        ..Style::default()
    };
    let shape = |oid: &str, geom, frame, fill, text: &str| {
        let mut o = Object::new_box(
            geom,
            frame,
            filled(fill),
            Label::new(text, 20.0, Align::Center),
        );
        o.id = id(oid);
        o
    };
    let mut doc = Document::new("Example board");
    let page = &mut doc.pages[0];
    page.id = id("gs-page-1");
    let mut title = Object::new_box(
        BoxGeom::Text,
        Frame::new(0.0, -150.0, 420.0, 40.0),
        Style::default(),
        Label::new("Welcome to your first board", 28.0, Align::Left),
    );
    title.id = id("gs-title");
    let collection = shape(
        "gs-collection",
        BoxGeom::Rect,
        Frame::new(0.0, 0.0, 220.0, 90.0),
        FILLS[5],
        "A collection",
    );
    let documents = shape(
        "gs-documents",
        BoxGeom::Rect,
        Frame::new(380.0, -80.0, 200.0, 80.0),
        FILLS[3],
        "Documents",
    );
    let boards = shape(
        "gs-boards",
        BoxGeom::Ellipse,
        Frame::new(380.0, 80.0, 200.0, 90.0),
        FILLS[6],
        "Boards",
    );
    let connect = |oid: &str, from: &Object, to: &Object, text: &str| {
        let mut c = Connector::new(End::bound(from.id.clone()), End::bound(to.id.clone()));
        c.label = Some(Label::new(text, 16.0, Align::Center));
        let mut o = Object::new(Kind::Connector(c), Style::default());
        o.id = id(oid);
        o
    };
    let to_documents = connect("gs-holds-documents", &collection, &documents, "holds");
    let to_boards = connect("gs-holds-boards", &collection, &boards, "and");
    let mut sticky = Object::new_box(
        BoxGeom::Sticky,
        Frame::new(0.0, 180.0, 260.0, 170.0),
        filled(NOTE_YELLOW),
        Label::new(
            "Drag the shapes around, double-click one to change its text, or draw with the pen.",
            18.0,
            Align::Left,
        ),
    );
    sticky.id = id("gs-sticky");
    page.objects = vec![
        title,
        collection,
        documents,
        boards,
        to_documents,
        to_boards,
        sticky,
    ];
    serial::to_json(&doc, false)
}

/// The envelope with every page id replaced, so two empty boards compare equal.
fn without_page_ids(envelope: &str) -> Value {
    let mut v: Value = serde_json::from_str(envelope).expect("envelope JSON");
    for page in v["document"]["pages"].as_array_mut().expect("pages") {
        page["id"] = Value::from("<page id>");
    }
    v
}

fn image(asset: &Id, x: f64) -> Object {
    Object::new(
        Kind::Image(ImageObj {
            frame: Frame::new(x, 0.0, 40.0, 40.0),
            asset: asset.clone(),
            latex: None,
            mono: false,
            recolor: Vec::new(),
        }),
        Style::default(),
    )
}

fn label_box(text: &str) -> Object {
    Object::new_box(
        BoxGeom::Rect,
        Frame::new(0.0, 0.0, 200.0, 80.0),
        Style::default(),
        Label::new(text, 20.0, Align::Center),
    )
}

/// A board with one image of `bytes` (and a label), as the engine writes it.
fn board_with_image(bytes: &[u8], text: &str) -> (Document, String) {
    let mut d = Document::new("client name");
    let a = d.add_asset("image/png", bytes.to_vec());
    d.pages[0]
        .objects
        .extend([label_box(text), image(&a, 300.0)]);
    let json = serial::to_json(&d, false);
    (d, json)
}

async fn new_board(t: &TestDb, owner: &str, title: &str) -> String {
    let c = t.collection(owner, "Boards").await;
    t.item(owner, c.id, None, ItemKind::Board, title)
        .await
        .id
        .to_string()
}

/// The PUT pipeline as the route runs it: `prepare` outside the writer, then `save` inside it.
async fn put(
    t: &TestDb,
    item: &str,
    user: &str,
    token: Option<&str>,
    body: &str,
    if_match: IfMatch,
) -> quire_store::Result<Saved> {
    let prepared = boards::prepare(&t.blobs, body, 0)?;
    let (blobs, item, user, token) = (
        t.blobs.clone(),
        item.to_owned(),
        user.to_owned(),
        token.map(str::to_owned),
    );
    t.db.write(move |tx| {
        let by = WriteBy {
            user_id: &user,
            token_id: token.as_deref(),
        };
        boards::save(
            tx,
            &blobs,
            &item,
            prepared,
            &if_match,
            &by,
            "test-engine",
            NOW,
        )
    })
    .await
}

/// What a GET returns: `stored` on a reader, then `assemble`.
async fn get(t: &TestDb, item: &str) -> String {
    let id = item.to_owned();
    let stored = t.db.read(move |c| boards::stored(c, &id)).await.unwrap();
    boards::assemble(&t.blobs, &stored).unwrap()
}

fn files_in(dir: &Path) -> usize {
    if !dir.exists() {
        return 0;
    }
    walk(dir)
}

fn walk(dir: &Path) -> usize {
    std::fs::read_dir(dir)
        .unwrap()
        .map(|e| {
            let e = e.unwrap();
            if e.file_type().unwrap().is_dir() {
                walk(&e.path())
            } else {
                1
            }
        })
        .sum()
}

#[tokio::test]
async fn getting_started_fixture_is_current_and_seeds() {
    let path = fixture_path();
    let expected = getting_started();
    let committed = std::fs::read_to_string(&path).ok();
    if std::env::var_os("WRITE_FIXTURES").is_some_and(|v| v == "1") {
        if committed.as_deref() != Some(expected.as_str()) {
            std::fs::create_dir_all(path.parent().unwrap()).unwrap();
            std::fs::write(&path, &expected).unwrap();
        }
        return;
    }
    let committed = committed
        .unwrap_or_else(|| panic!("{} is missing; run with WRITE_FIXTURES=1", path.display()));
    assert!(
        committed == expected,
        "{} differs from the code; run with WRITE_FIXTURES=1",
        path.display()
    );
    assert!(
        !committed.ends_with('\n'),
        "the fixture holds the exact stored bytes"
    );
    // A few labelled shapes, connectors and a sticky note (§9.6.7).
    let doc = serial::from_json(&committed).unwrap();
    let kinds: Vec<&Kind> = doc.pages[0].objects.iter().map(|o| &o.kind).collect();
    assert!(
        kinds
            .iter()
            .any(|k| matches!(k, Kind::Box(b) if b.geom == BoxGeom::Sticky))
    );
    assert_eq!(
        kinds
            .iter()
            .filter(|k| matches!(k, Kind::Connector(_)))
            .count(),
        2
    );
    // The seed path stores it byte for byte, and indexes its labels.
    let t = TestDb::new();
    let owner = t.owner().await;
    let id = new_board(&t, &owner, "Example board").await;
    let (i, blobs, env) = (id.clone(), t.blobs.clone(), committed.clone());
    let stored =
        t.db.write(move |tx| {
            boards::init_with(tx, &blobs, &i, &env, NOW)?;
            boards::stored_envelope(tx, &i)
        })
        .await
        .unwrap();
    assert_eq!(stored, committed);
    let body: String = t
        .query(move |c| {
            c.query_row(
                "SELECT body FROM item_text WHERE item_id = ?1",
                [&id],
                |r| r.get(0),
            )
        })
        .await;
    for label in [
        "A collection",
        "Documents",
        "Boards",
        "holds",
        "double-click",
    ] {
        assert!(body.contains(label), "{label} in {body:?}");
    }
}

#[tokio::test]
async fn new_board_is_the_empty_envelope_with_meta() {
    let t = TestDb::new();
    let owner = t.owner().await;
    let c = t.collection(&owner, "Notes").await;
    let light = t.item(&owner, c.id, None, ItemKind::Board, "Light").await;
    let id = light.id.to_string();
    let (envelope, meta, sha, raw_len, format_version, engine) =
        t.db.read(move |c| {
            let (sha, raw_len, format_version, engine): (Vec<u8>, i64, i64, String) = c.query_row(
                "SELECT body_sha256, raw_len, format_version, engine_version FROM board_state WHERE item_id = ?1",
                [&id],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)),
            )?;
            Ok((
                boards::stored_envelope(c, &id)?,
                boards::meta(c, &id)?,
                sha,
                raw_len,
                format_version,
                engine,
            ))
        })
        .await
        .unwrap();
    assert_eq!(
        without_page_ids(&envelope),
        without_page_ids(&boards::empty_envelope(false))
    );
    assert_eq!(raw_len as usize, envelope.len());
    assert_eq!((format_version, engine.as_str()), (2, "server"));
    use sha2::Digest;
    assert_eq!(sha, sha2::Sha256::digest(envelope.as_bytes()).to_vec());
    let doc: Value = serde_json::from_str(&envelope).unwrap();
    assert_eq!(meta.pages.len(), 1);
    assert_eq!(
        meta.pages[0].id,
        doc["document"]["pages"][0]["id"].as_str().unwrap()
    );
    assert_eq!(meta.pages[0].name, "Page 1");
    assert_eq!((meta.objects, meta.background.as_str()), (0, "#ffffff"));
    assert_eq!(meta.bytes as usize, envelope.len());
    assert_eq!(meta.text_excerpt, "Page 1");
    let id = light.id.to_string();
    assert!(
        t.db.write(move |tx| boards::is_empty(tx, &id))
            .await
            .unwrap()
    );
    assert_eq!(
        count(
            &t,
            "SELECT count(*) FROM item_text WHERE item_id = ?1 AND body = 'Page 1'",
            light.id.to_string()
        )
        .await,
        1
    );

    let (o, cid) = (owner.clone(), c.id);
    let dark =
        t.db.write(move |tx| {
            quire_store::items::create(
                tx,
                &o,
                &quire_core::api::NewItem {
                    collection_id: cid,
                    parent_id: None,
                    kind: ItemKind::Board,
                    title: None,
                    position: None,
                },
                true,
                NOW,
            )
        })
        .await
        .unwrap();
    let id = dark.id.to_string();
    let (env, meta) =
        t.db.read(move |c| Ok((boards::stored_envelope(c, &id)?, boards::meta(c, &id)?)))
            .await
            .unwrap();
    let v: Value = serde_json::from_str(&env).unwrap();
    assert_eq!(v["document"]["background"]["preset"], "slate");
    assert_eq!(meta.background, "#262b38");
}

#[tokio::test]
async fn init_with_validates_and_splits_the_envelope() {
    let t = TestDb::new();
    let owner = t.owner().await;
    let c = t.collection(&owner, "Notes").await;
    let board = t.item(&owner, c.id, None, ItemKind::Board, "B").await;
    let doc = t.item(&owner, c.id, None, ItemKind::Doc, "D").await;
    let attempt = |item: String, env: String| {
        let blobs = t.blobs.clone();
        t.db.write(move |tx| boards::init_with(tx, &blobs, &item, &env, NOW))
    };
    let e = attempt(board.id.to_string(), "{".into()).await.unwrap_err();
    assert!(e.is(ErrorCode::InvalidBoard));
    let e = attempt(
        board.id.to_string(),
        r#"{"format":"excalidraw","version":2,"document":{}}"#.into(),
    )
    .await
    .unwrap_err();
    assert!(e.is(ErrorCode::InvalidBoard));
    let newer = boards::empty_envelope(false).replace(r#""version":2"#, r#""version":3"#);
    let e = attempt(board.id.to_string(), newer).await.unwrap_err();
    assert!(e.is(ErrorCode::BoardTooNew), "{e:?}");
    let e = attempt(doc.id.to_string(), boards::empty_envelope(false))
        .await
        .unwrap_err();
    assert!(e.is(ErrorCode::Validation));
    // A board with an object and an inline image: rev 1 again, the image in the blob store, the body stripped.
    let (_, env) = board_with_image(PNG, "Seeded label");
    let id = board.id.to_string();
    t.db.write(move |tx| {
        tx.execute("UPDATE board_state SET rev = 7 WHERE item_id = ?1", [&id])?;
        Ok(())
    })
    .await
    .unwrap();
    attempt(board.id.to_string(), env.clone()).await.unwrap();
    let id = board.id.to_string();
    let (meta, rev, stripped, assets) =
        t.db.read(move |c| {
            let rev: i64 = c.query_row(
                "SELECT rev FROM board_state WHERE item_id = ?1",
                [&id],
                |r| r.get(0),
            )?;
            let assets: i64 = c.query_row(
                "SELECT count(*) FROM board_assets WHERE item_id = ?1",
                [&id],
                |r| r.get(0),
            )?;
            Ok((
                boards::meta(c, &id)?,
                rev,
                boards::stored_envelope(c, &id)?,
                assets,
            ))
        })
        .await
        .unwrap();
    assert_eq!((meta.objects, rev, assets), (2, 1, 1));
    assert!(stripped.contains(r#""data":"""#));
    assert_eq!(files_in(&t.data_dir().join("files")), 1);
    let id = board.id.to_string();
    assert!(
        !t.db
            .write(move |tx| boards::is_empty(tx, &id))
            .await
            .unwrap()
    );
    // The GET path gives the envelope back, with the item's title as the name.
    let full = get(&t, &board.id.to_string()).await;
    let mut expected: Value = serde_json::from_str(&env).unwrap();
    expected["document"]["name"] = "B".into();
    assert_eq!(serde_json::from_str::<Value>(&full).unwrap(), expected);
}

#[tokio::test]
async fn copy_shares_blobs() {
    let t = TestDb::new();
    let owner = t.owner().await;
    let from = new_board(&t, &owner, "From").await;
    let to = new_board(&t, &owner, "To").await;
    let (_, body) = board_with_image(PNG, "Shared");
    put(&t, &from, &owner, Some("tok"), &body, IfMatch::Any)
        .await
        .unwrap();
    let (f, tid) = (from.clone(), to.clone());
    t.db.write(move |tx| {
        tx.execute(
            "INSERT INTO board_previews (item_id, page_index, rev, svg, svg_len) VALUES (?1, 0, 1, NULL, 0)",
            [&tid],
        )?;
        boards::copy(tx, &f, &tid, NOW)
    })
    .await
    .unwrap();
    let tid = to.clone();
    let (rev, last_token, assets, previews, same_body, blobs): (
        i64,
        Option<String>,
        i64,
        i64,
        bool,
        i64,
    ) = t
        .query(move |c| {
            c.query_row(
                "SELECT s.rev, s.last_token_id,
                   (SELECT count(*) FROM board_assets a WHERE a.item_id = s.item_id
                      AND a.sha256 = (SELECT sha256 FROM board_assets WHERE item_id = ?2)),
                   (SELECT count(*) FROM board_previews p WHERE p.item_id = s.item_id),
                   s.body = (SELECT body FROM board_state f WHERE f.item_id = ?2),
                   (SELECT count(*) FROM blobs)
                 FROM board_state s WHERE s.item_id = ?1",
                params![tid, from],
                |r| {
                    Ok((
                        r.get(0)?,
                        r.get(1)?,
                        r.get(2)?,
                        r.get(3)?,
                        r.get(4)?,
                        r.get(5)?,
                    ))
                },
            )
        })
        .await;
    assert_eq!(
        (rev, last_token, assets, previews, same_body, blobs),
        (1, None, 1, 0, true, 1)
    );
    assert_eq!(files_in(&t.data_dir().join("files")), 1);
    // The copy reads back with its own title and the shared image bytes.
    let full = get(&t, &to).await;
    let doc = serial::from_json(&full).unwrap();
    assert_eq!(doc.name, "To");
    assert_eq!(doc.assets.values().next().unwrap().data.as_slice(), PNG);
    assert_eq!(
        count(
            &t,
            "SELECT count(*) FROM item_text WHERE item_id = ?1 AND body LIKE '%Shared%'",
            to
        )
        .await,
        1
    );
}

#[tokio::test]
async fn assets_deduped_across_boards() {
    let t = TestDb::new();
    let owner = t.owner().await;
    let a = new_board(&t, &owner, "A").await;
    let b = new_board(&t, &owner, "B").await;
    // Board A: the PNG twice under two asset ids (the engine dedupes only identical type and bytes) plus a GIF.
    let mut d = Document::new("");
    let png = d.add_asset("image/png", PNG.to_vec());
    let png_again = d.add_asset("image/x-png", PNG.to_vec());
    let gif = d.add_asset("image/gif", GIF.to_vec());
    d.pages[0].objects.extend([
        image(&png, 0.0),
        image(&png_again, 50.0),
        image(&gif, 100.0),
    ]);
    put(
        &t,
        &a,
        &owner,
        None,
        &serial::to_json(&d, false),
        IfMatch::Any,
    )
    .await
    .unwrap();
    // Board B: the same PNG.
    let (_, body) = board_with_image(PNG, "B's label");
    put(&t, &b, &owner, None, &body, IfMatch::Any)
        .await
        .unwrap();
    let (rows, blobs, distinct): (i64, i64, i64) = t
        .query(|c| {
            c.query_row(
                "SELECT (SELECT count(*) FROM board_assets), (SELECT count(*) FROM blobs),
                   (SELECT count(DISTINCT sha256) FROM board_assets)",
                [],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
            )
        })
        .await;
    assert_eq!((rows, blobs, distinct), (4, 2, 2));
    assert_eq!(files_in(&t.data_dir().join("files")), 2);
    assert_eq!(files_in(&t.data_dir().join("tmp")), 0);
    // The blob rows carry the sniffed type; board_assets the envelope's.
    let mime: String = t
        .query(move |c| {
            c.query_row(
                "SELECT mime FROM board_assets WHERE item_id = ?1 AND asset_id = ?2",
                params![a, png_again.as_str()],
                |r| r.get(0),
            )
        })
        .await;
    assert_eq!(mime, "image/x-png");
    assert_eq!(
        count(
            &t,
            "SELECT count(*) FROM blobs WHERE mime = ?1",
            "image/png".into()
        )
        .await,
        1
    );
}

#[tokio::test]
async fn noop_save_keeps_rev_and_drops_staged_files() {
    let t = TestDb::new();
    let owner = t.owner().await;
    let id = new_board(&t, &owner, "Board").await;
    let (_, body) = board_with_image(PNG, "Same");
    let first = put(&t, &id, &owner, None, &body, IfMatch::Revs(vec![1]))
        .await
        .unwrap();
    assert!(first.changed);
    assert_eq!(first.rev, 2);
    // The same body again, even with a stale If-Match: nothing is written, rev stays.
    let prepared = boards::prepare(&t.blobs, &body, 0).unwrap();
    assert_eq!(prepared.staged_assets(), 1);
    assert_eq!(files_in(&t.data_dir().join("tmp")), 1, "staged");
    let (blobs, i, o) = (t.blobs.clone(), id.clone(), owner.clone());
    let again =
        t.db.write(move |tx| {
            let by = WriteBy {
                user_id: &o,
                token_id: None,
            };
            boards::save(
                tx,
                &blobs,
                &i,
                prepared,
                &IfMatch::Revs(vec![1]),
                &by,
                "e",
                NOW + MINUTE,
            )
        })
        .await
        .unwrap();
    assert!(!again.changed);
    assert_eq!((again.rev, again.updated_at), (2, NOW));
    assert_eq!(
        files_in(&t.data_dir().join("tmp")),
        0,
        "the staged file is gone"
    );
    assert_eq!(files_in(&t.data_dir().join("files")), 1);
    let (rev, revisions, engine): (i64, i64, String) = t
        .query(move |c| {
            c.query_row(
                "SELECT rev, (SELECT count(*) FROM revisions WHERE item_id = ?1), engine_version
                 FROM board_state WHERE item_id = ?1",
                [&id],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
            )
        })
        .await;
    assert_eq!((rev, revisions, engine.as_str()), (2, 1, "test-engine"));
}

#[tokio::test]
async fn save_sets_body_text_with_box_label() {
    let t = TestDb::new();
    let owner = t.owner().await;
    let id = new_board(&t, &owner, "Planning").await;
    let mut d = Document::new("");
    d.pages[0].name = "Roadmap".into();
    d.pages[0].objects.push(label_box("Quarterly hyperplan"));
    let saved = put(
        &t,
        &id,
        &owner,
        None,
        &serial::to_json(&d, false),
        IfMatch::Any,
    )
    .await
    .unwrap();
    assert_eq!(saved.meta.text_excerpt, "Roadmap\nQuarterly hyperplan");
    assert_eq!(saved.meta.objects, 1);
    assert_eq!(saved.meta.pages[0].name, "Roadmap");
    let i = id.clone();
    let (body, hits, content_at, updated_by): (String, i64, Option<i64>, String) = t
        .query(move |c| {
            c.query_row(
                "SELECT t.body,
                   (SELECT count(*) FROM item_fts WHERE item_fts MATCH 'hyperplan'),
                   i.content_at, i.updated_by
                 FROM item_text t JOIN items i ON i.id = t.item_id WHERE t.item_id = ?1",
                [&i],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)),
            )
        })
        .await;
    assert_eq!(body, "Roadmap\nQuarterly hyperplan");
    assert_eq!((hits, content_at, updated_by), (1, Some(NOW), owner));
}

#[tokio::test]
async fn save_checks_if_match_and_keeps_revisions() {
    let t = TestDb::new();
    let owner = t.owner().await;
    let id = new_board(&t, &owner, "Board").await;
    let body = |text: &str| board_with_image(PNG, text).1;
    // rev 1 → 2: the first save writes an `auto` revision with the asset manifest.
    put(&t, &id, &owner, None, &body("one"), IfMatch::Revs(vec![1]))
        .await
        .unwrap();
    // A stale If-Match: 412 with the current rev and etag.
    let e = put(&t, &id, &owner, None, &body("two"), IfMatch::Revs(vec![1]))
        .await
        .unwrap_err();
    assert!(e.is(ErrorCode::RevMismatch), "{e:?}");
    assert_eq!(
        e.details,
        Some(serde_json::json!({"etag": "\"r2\"", "rev": 2}))
    );
    assert_eq!(files_in(&t.data_dir().join("tmp")), 0);
    // A tag that names no board revision never matches; `*` always does.
    let e = put(&t, &id, &owner, None, &body("two"), IfMatch::Revs(vec![]))
        .await
        .unwrap_err();
    assert!(e.is(ErrorCode::RevMismatch));
    let s = put(&t, &id, &owner, None, &body("two"), IfMatch::Any)
        .await
        .unwrap();
    assert_eq!(s.rev, 3);
    let i = id.clone();
    let (reason, source, assets, author): (String, i64, String, String) = t
        .query(move |c| {
            c.query_row(
                "SELECT reason, source_seq, assets, author_id FROM revisions WHERE item_id = ?1",
                [&i],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)),
            )
        })
        .await;
    assert_eq!((reason.as_str(), source), ("auto", 2));
    let assets: Value = serde_json::from_str(&assets).unwrap();
    assert_eq!(
        assets.as_object().unwrap().values().next().unwrap(),
        &Value::from(quire_store::Sha256::of(PNG).as_str())
    );
    assert_eq!(author, owner);
    // A token's write after the session's: one `pre-api` revision of rev 3; its next write: none (D53).
    put(&t, &id, &owner, Some("tok-1"), &body("three"), IfMatch::Any)
        .await
        .unwrap();
    put(&t, &id, &owner, Some("tok-1"), &body("four"), IfMatch::Any)
        .await
        .unwrap();
    let i = id.clone();
    let (pre_api, source, last_token): (i64, i64, Option<String>) = t
        .query(move |c| {
            c.query_row(
                "SELECT count(*), max(source_seq),
                   (SELECT last_token_id FROM board_state WHERE item_id = ?1)
                 FROM revisions WHERE item_id = ?1 AND reason = 'pre-api'",
                [&i],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
            )
        })
        .await;
    assert_eq!(
        (pre_api, source, last_token.as_deref()),
        (1, 3, Some("tok-1"))
    );
    // A session's write clears last_token_id.
    put(&t, &id, &owner, None, &body("five"), IfMatch::Any)
        .await
        .unwrap();
    assert_eq!(
        count(
            &t,
            "SELECT count(*) FROM board_state WHERE item_id = ?1 AND last_token_id IS NULL AND rev = 6",
            id
        )
        .await,
        1
    );
}

#[tokio::test]
async fn empty_asset_data_keeps_the_previous_blob() {
    let t = TestDb::new();
    let owner = t.owner().await;
    let id = new_board(&t, &owner, "Board").await;
    let (_, full) = board_with_image(PNG, "with image");
    put(&t, &id, &owner, None, &full, IfMatch::Any)
        .await
        .unwrap();
    // The same body without its data, plus a new label: the image keeps its blob.
    let split = quire_core::board::split(&full).unwrap();
    let edited = split.stripped.replace("with image", "with image, edited");
    put(&t, &id, &owner, None, &edited, IfMatch::Any)
        .await
        .unwrap();
    let back = serial::from_json(&get(&t, &id).await).unwrap();
    assert_eq!(back.assets.values().next().unwrap().data.as_slice(), PNG);
    // An empty asset the board never had stays without data and gets no row.
    let other = new_board(&t, &owner, "Other").await;
    put(&t, &other, &owner, None, &edited, IfMatch::Any)
        .await
        .unwrap();
    assert_eq!(
        count(
            &t,
            "SELECT count(*) FROM board_assets WHERE item_id = ?1",
            other.clone()
        )
        .await,
        0
    );
    let back: Value = serde_json::from_str(&get(&t, &other).await).unwrap();
    let asset = back["document"]["assets"]
        .as_object()
        .unwrap()
        .values()
        .next()
        .unwrap();
    assert_eq!(asset["data"], "");
}

#[tokio::test]
async fn save_refuses_documents_trashed_boards_and_bad_bodies() {
    let t = TestDb::new();
    let owner = t.owner().await;
    let c = t.collection(&owner, "Notes").await;
    let doc = t.item(&owner, c.id, None, ItemKind::Doc, "D").await.id;
    let board = new_board(&t, &owner, "B").await;
    let body = boards::empty_envelope(false);
    let e = put(&t, &doc.to_string(), &owner, None, &body, IfMatch::Any)
        .await
        .unwrap_err();
    assert!(e.is(ErrorCode::Validation));
    let e = put(
        &t,
        &uuid::Uuid::now_v7().to_string(),
        &owner,
        None,
        &body,
        IfMatch::Any,
    )
    .await
    .unwrap_err();
    assert!(e.is(ErrorCode::NotFound));
    let e = boards::prepare(&t.blobs, "{\"format\":\"inkboard\"}", 0).unwrap_err();
    assert!(e.is(ErrorCode::InvalidBoard));
    let e =
        boards::prepare(&t.blobs, &body.replace("\"version\":2", "\"version\":9"), 0).unwrap_err();
    assert!(e.is(ErrorCode::BoardTooNew));
    let (_, with_image) = board_with_image(PNG, "x");
    let b = board.clone();
    t.db.write(move |tx| {
        tx.execute(
            "UPDATE items SET deleted_at = ?2, trash_root_id = id WHERE id = ?1",
            params![b, NOW],
        )?;
        Ok(())
    })
    .await
    .unwrap();
    let e = put(&t, &board, &owner, None, &with_image, IfMatch::Any)
        .await
        .unwrap_err();
    assert!(e.is(ErrorCode::ReadOnly));
}

#[tokio::test]
async fn previews_are_stored_per_page_and_revision() {
    let t = TestDb::new();
    let owner = t.owner().await;
    let id = new_board(&t, &owner, "Board").await;
    let mut d = Document::new("");
    d.pages[0].objects.push(label_box("first"));
    d.pages.push(inkboard_core::model::Page::new("Second"));
    let saved = put(
        &t,
        &id,
        &owner,
        None,
        &serial::to_json(&d, false),
        IfMatch::Any,
    )
    .await
    .unwrap();
    assert_eq!(saved.meta.preview_rev, None);
    assert!(saved.meta.pages.iter().all(|p| p.svg_len.is_none()));
    let full = get(&t, &id).await;
    let svgs = quire_core::board::previews(&full).unwrap();
    assert!(svgs[0].is_some() && svgs[1].is_none());
    // A render of an older revision is refused.
    let (i, s) = (id.clone(), svgs.clone());
    assert!(
        !t.db
            .write(move |tx| previews::replace(tx, &i, 1, &s))
            .await
            .unwrap()
    );
    let (i, s) = (id.clone(), svgs.clone());
    assert!(
        t.db.write(move |tx| previews::replace(tx, &i, 2, &s))
            .await
            .unwrap()
    );
    let i = id.clone();
    let (page0, page1, stale, rev, meta) =
        t.db.read(move |c| {
            Ok((
                previews::get(c, &i, 0, 2)?,
                previews::get(c, &i, 1, 2)?,
                previews::get(c, &i, 0, 1)?,
                previews::rev(c, &i)?,
                boards::board_meta(c, &i)?,
            ))
        })
        .await
        .unwrap();
    assert_eq!(
        page0,
        Some(previews::Preview::Svg(svgs[0].clone().unwrap()))
    );
    assert_eq!(page1, Some(previews::Preview::Empty));
    assert_eq!((stale, rev), (None, Some(2)));
    assert_eq!(meta.preview_rev, Some(2));
    assert_eq!(meta.rev, 2);
    assert_eq!(
        meta.pages[0].svg_len,
        Some(svgs[0].as_ref().unwrap().len() as u64)
    );
    assert_eq!(meta.pages[1].svg_len, Some(0));
    // Fewer pages after a save: the extra rows go with the next render.
    let mut one = Document::new("");
    one.pages[0].objects.push(label_box("only"));
    put(
        &t,
        &id,
        &owner,
        None,
        &serial::to_json(&one, false),
        IfMatch::Any,
    )
    .await
    .unwrap();
    let svgs = quire_core::board::previews(&get(&t, &id).await).unwrap();
    let i = id.clone();
    t.db.write(move |tx| previews::replace(tx, &i, 3, &svgs))
        .await
        .unwrap();
    assert_eq!(
        count(
            &t,
            "SELECT count(*) FROM board_previews WHERE item_id = ?1",
            id.clone()
        )
        .await,
        1
    );
    let state = t.db.read(move |c| boards::state(c, &id)).await.unwrap();
    assert_eq!((state.rev, state.pages, state.updated_at), (3, 1, NOW));
}

#[tokio::test]
async fn asset_bytes_count_in_the_noop_test() {
    use std::collections::HashMap;
    let t = TestDb::new();
    let owner = t.owner().await;
    let id = new_board(&t, &owner, "Board").await;
    let (_, first) = board_with_image(PNG, "Same");
    assert_eq!(
        put(&t, &id, &owner, None, &first, IfMatch::Any)
            .await
            .unwrap()
            .rev,
        2
    );
    // The same envelope with other bytes under the same asset id: a change, not a no-op.
    let split = quire_core::board::split(&first).unwrap();
    let asset = split.assets[0].id.clone();
    let other = quire_core::board::splice(
        &split.stripped,
        &HashMap::from([(asset.clone(), GIF.to_vec())]),
        "client name",
    )
    .unwrap();
    let saved = put(&t, &id, &owner, None, &other, IfMatch::Revs(vec![2]))
        .await
        .unwrap();
    assert!(saved.changed);
    assert_eq!(saved.rev, 3);
    let back = serial::from_json(&get(&t, &id).await).unwrap();
    assert_eq!(back.assets[&Id::from(asset.as_str())].data.as_slice(), GIF);
    // The same bytes again: a no-op.
    let again = put(&t, &id, &owner, None, &other, IfMatch::Revs(vec![1]))
        .await
        .unwrap();
    assert!(!again.changed);
    assert_eq!(again.rev, 3);
    // The GET's length is reserved before the envelope is assembled: the estimate covers it.
    let (i, full) = (id.clone(), get(&t, &id).await);
    let estimate =
        t.db.read(move |c| boards::envelope_len(c, &i))
            .await
            .unwrap();
    assert!(
        estimate as usize >= full.len() && estimate as usize <= full.len() + 64,
        "{estimate} for {}",
        full.len()
    );

    // An asset stored without data, sent again with its bytes: saved.
    let empty = new_board(&t, &owner, "Empty").await;
    put(&t, &empty, &owner, None, &split.stripped, IfMatch::Any)
        .await
        .unwrap();
    let saved = put(&t, &empty, &owner, None, &first, IfMatch::Any)
        .await
        .unwrap();
    assert!(saved.changed);
    assert_eq!(saved.rev, 3);
    let back = serial::from_json(&get(&t, &empty).await).unwrap();
    assert_eq!(back.assets[&Id::from(asset.as_str())].data.as_slice(), PNG);
    // A blob file that went missing: sending the bytes again restores it.
    let file = t.blobs.path(&quire_store::blobs::Sha256::of(PNG));
    std::fs::remove_file(&file).unwrap();
    let saved = put(&t, &empty, &owner, None, &first, IfMatch::Any)
        .await
        .unwrap();
    assert!(saved.changed);
    assert_eq!(saved.rev, 4);
    assert!(file.exists());
}

#[tokio::test]
async fn pages_after_the_preview_budget_are_omitted() {
    let t = TestDb::new();
    let owner = t.owner().await;
    let id = new_board(&t, &owner, "Board").await;
    let mut d = Document::new("");
    d.pages[0].objects.push(label_box("first"));
    for name in ["Second", "Third"] {
        let mut page = inkboard_core::model::Page::new(name);
        page.objects.push(label_box(name));
        d.pages.push(page);
    }
    put(
        &t,
        &id,
        &owner,
        None,
        &serial::to_json(&d, false),
        IfMatch::Any,
    )
    .await
    .unwrap();
    let svg = quire_core::board::preview(&get(&t, &id).await, 0)
        .unwrap()
        .unwrap();
    let pages = vec![Some(previews::PageSvg::new(&svg).unwrap())];
    let i = id.clone();
    assert!(
        t.db.write(move |tx| previews::store(tx, &i, 2, &pages))
            .await
            .unwrap()
    );
    let i = id.clone();
    let (rendered, first, second, third, stale, meta) =
        t.db.read(move |c| {
            Ok((
                previews::rendered(c, &i, 2)?,
                previews::state(c, &i, 0, 2)?,
                previews::state(c, &i, 1, 2)?,
                previews::get(c, &i, 2, 2)?,
                previews::state(c, &i, 1, 1)?,
                boards::board_meta(c, &i)?,
            ))
        })
        .await
        .unwrap();
    assert!(rendered);
    assert_eq!(first, Some(previews::PageState::Svg { len: svg.len() }));
    assert_eq!(second, Some(previews::PageState::Omitted));
    assert_eq!(third, Some(previews::Preview::Omitted));
    assert_eq!(stale, None, "rev 1 was never rendered");
    assert_eq!(meta.pages[0].svg_len, Some(svg.len() as u64));
    assert_eq!(meta.pages[1].svg_len, None);
    let state = t.db.read(move |c| boards::state(c, &id)).await.unwrap();
    assert_eq!(state.background, "#ffffff");
}
