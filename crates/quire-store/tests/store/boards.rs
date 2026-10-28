//! Board storage tests (owner: W6)
//!
//! W0b: the initializers (empty light and dark boards, a given envelope, copy), `is_empty`, and the seed fixture
//! `fixtures/boards/getting-started.inkboard`, which is the envelope `boards::init_empty(.., dark = false, ..)` stores.
//! `WRITE_FIXTURES=1 scripts/dev.sh cargo test -p quire-store --locked --test store -- boards::` rewrites the fixture
//! when its content (ignoring the random page id) differs from what the code produces.

use std::path::PathBuf;

use quire_core::api::{ErrorCode, ItemKind};
use quire_store::boards;
use rusqlite::params;
use serde_json::Value;

use crate::common::{NOW, TestDb, count};

fn fixture_path() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../fixtures/boards/getting-started.inkboard")
}

/// The envelope with every page id replaced, so two empty boards compare equal.
fn without_page_ids(envelope: &str) -> Value {
    let mut v: Value = serde_json::from_str(envelope).expect("envelope JSON");
    for page in v["document"]["pages"].as_array_mut().expect("pages") {
        page["id"] = Value::from("<page id>");
    }
    v
}

#[tokio::test]
async fn getting_started_fixture_is_the_empty_light_board() {
    let path = fixture_path();
    let expected = boards::empty_envelope(false);
    let committed = std::fs::read_to_string(&path).ok();
    let same = committed
        .as_deref()
        .is_some_and(|c| without_page_ids(c) == without_page_ids(&expected));
    if std::env::var_os("WRITE_FIXTURES").is_some_and(|v| v == "1") {
        if !same {
            std::fs::create_dir_all(path.parent().unwrap()).unwrap();
            std::fs::write(&path, &expected).unwrap();
        }
        return;
    }
    let committed = committed
        .unwrap_or_else(|| panic!("{} is missing; run with WRITE_FIXTURES=1", path.display()));
    assert!(
        same,
        "{} differs from boards::empty_envelope(false); run with WRITE_FIXTURES=1",
        path.display()
    );
    assert!(
        !committed.ends_with('\n'),
        "the fixture holds the exact stored bytes"
    );
    // The seed path accepts it, and stores it byte for byte.
    let t = TestDb::new();
    let owner = t.owner().await;
    let c = t.collection(&owner, "Getting started").await;
    let board = t
        .item(&owner, c.id, None, ItemKind::Board, "Whiteboard")
        .await;
    let (id, blobs, env) = (board.id.to_string(), t.blobs.clone(), committed.clone());
    let stored =
        t.db.write(move |tx| {
            boards::init_with(tx, &blobs, &id, &env, NOW)?;
            boards::stored_envelope(tx, &id)
        })
        .await
        .unwrap();
    assert_eq!(stored, committed);
}

#[tokio::test]
async fn new_board_is_the_empty_envelope_with_meta() {
    let t = TestDb::new();
    let owner = t.owner().await;
    let c = t.collection(&owner, "Notes").await;
    let light = t.item(&owner, c.id, None, ItemKind::Board, "Light").await;
    let id = light.id.to_string();
    let (envelope, meta, sha, raw_len, format_version) =
        t.db.read(move |c| {
            let (sha, raw_len, format_version): (Vec<u8>, i64, i64) = c.query_row(
                "SELECT body_sha256, raw_len, format_version FROM board_state WHERE item_id = ?1",
                [&id],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
            )?;
            Ok((
                boards::stored_envelope(c, &id)?,
                boards::meta(c, &id)?,
                sha,
                raw_len,
                format_version,
            ))
        })
        .await
        .unwrap();
    assert_eq!(
        without_page_ids(&envelope),
        without_page_ids(&boards::empty_envelope(false))
    );
    assert_eq!(raw_len as usize, envelope.len());
    assert_eq!(format_version, 2);
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
async fn init_with_validates_the_envelope() {
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
    // A board with an object: stored as given, rev 1 again, not empty.
    let mut d = inkboard_core::model::Document::new("ignored name");
    d.pages[0]
        .objects
        .push(inkboard_core::model::Object::new_pen(
            vec![
                inkboard_core::Pt { x: 0.0, y: 0.0 },
                inkboard_core::Pt { x: 10.0, y: 10.0 },
            ],
            false,
            inkboard_core::Style::default(),
        ));
    let env = inkboard_core::serial::to_json(&d, false);
    let id = board.id.to_string();
    t.db.write(move |tx| {
        tx.execute("UPDATE board_state SET rev = 7 WHERE item_id = ?1", [&id])?;
        Ok(())
    })
    .await
    .unwrap();
    attempt(board.id.to_string(), env.clone()).await.unwrap();
    let id = board.id.to_string();
    let (meta, rev, stored) =
        t.db.read(move |c| {
            let rev: i64 = c.query_row(
                "SELECT rev FROM board_state WHERE item_id = ?1",
                [&id],
                |r| r.get(0),
            )?;
            Ok((boards::meta(c, &id)?, rev, boards::stored_envelope(c, &id)?))
        })
        .await
        .unwrap();
    assert_eq!((meta.objects, rev), (1, 1));
    assert_eq!(stored, env);
    let id = board.id.to_string();
    assert!(
        !t.db
            .write(move |tx| boards::is_empty(tx, &id))
            .await
            .unwrap()
    );
    // Inline asset data needs W6's split.
    let mut d = inkboard_core::model::Document::new("");
    d.add_asset("image/png", b"\x89PNG\r\n\x1a\n".to_vec());
    let e = attempt(
        board.id.to_string(),
        inkboard_core::serial::to_json(&d, false),
    )
    .await
    .unwrap_err();
    assert!(e.is(ErrorCode::InvalidBoard));
}

#[tokio::test]
async fn copy_shares_blobs() {
    let t = TestDb::new();
    let owner = t.owner().await;
    let c = t.collection(&owner, "Notes").await;
    let from = t.item(&owner, c.id, None, ItemKind::Board, "From").await;
    let to = t.item(&owner, c.id, None, ItemKind::Board, "To").await;
    let (f, tid) = (from.id.to_string(), to.id.to_string());
    let blobs = t.blobs.clone();
    t.db.write(move |tx| {
        let blob = blobs.put(tx, b"\x89PNG\r\n\x1a\n", NOW)?;
        tx.execute(
            "INSERT INTO board_assets (item_id, asset_id, sha256, mime) VALUES (?1, 'asset1', ?2, 'image/png')",
            params![f, blob.sha256.as_str()],
        )?;
        tx.execute("UPDATE board_state SET rev = 5, last_token_id = 'tok' WHERE item_id = ?1", [&f])?;
        tx.execute(
            "INSERT INTO board_previews (item_id, page_index, rev, svg, svg_len) VALUES (?1, 0, 1, NULL, 0)",
            [&tid],
        )?;
        boards::copy(tx, &f, &tid, NOW)
    })
    .await
    .unwrap();
    let tid = to.id.to_string();
    let (rev, last_token, shared_assets, previews, same_body): (i64, Option<String>, i64, i64, bool) = t
        .query(move |c| {
            c.query_row(
                "SELECT s.rev, s.last_token_id,
                   (SELECT count(*) FROM board_assets a WHERE a.item_id = s.item_id AND a.asset_id = 'asset1'),
                   (SELECT count(*) FROM board_previews p WHERE p.item_id = s.item_id),
                   s.body = (SELECT body FROM board_state f WHERE f.item_id = ?2)
                 FROM board_state s WHERE s.item_id = ?1",
                params![tid, from.id.to_string()],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?)),
            )
        })
        .await;
    assert_eq!(
        (rev, last_token, shared_assets, previews, same_body),
        (1, None, 1, 0, true)
    );
    assert_eq!(
        count(
            &t,
            "SELECT count(*) FROM blobs WHERE ?1 = ''",
            String::new()
        )
        .await,
        1
    );
}
